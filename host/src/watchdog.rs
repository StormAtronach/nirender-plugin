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
//!
//! What a stall leaves to read. The frame thread names the phase of its step with one relaxed
//! store at each change ([`phase`]). From [`SAMPLE_FROM_MS`] into a step, a second thread (the
//! sampler) reads the frame thread's instruction pointer at each look and keeps the last
//! [`SAMPLES_KEPT`]. The sampler is a thread of its own, so that a frame thread that cannot be
//! suspended (a wait of the kernel that holds off the suspend) never keeps the watchdog from
//! ending the program. The stall line and the dump carry the phase and the samples. The
//! sampler runs only inside a step that is already slow: in a step of a few milliseconds it
//! never takes a sample.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::crash::{StallEvidence, SAMPLES_KEPT};
use crate::stream_format::{EXIT_STALLED, LOADING_SECONDS, STALL_SECONDS};

/// The time between two looks of the watchdog thread.
const LOOK_MS: u64 = 250;
/// The time into a step from which the sampler takes a sample at each look.
const SAMPLE_FROM_MS: u64 = 1000;

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
/// The phase of the frame thread's step ([`Phase`]).
static PHASE: AtomicU8 = AtomicU8::new(Phase::Feed as u8);
/// The watchdog thread says that the step is slow enough for the sampler to take samples.
static SAMPLING: AtomicBool = AtomicBool::new(false);
/// What the sampler read in the step that is in progress.
static RING: SampleRing = SampleRing::new();

/// Where in its step the frame thread is, for the line of a stall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Phase {
    /// The feed of the record: the start of every step.
    Feed,
    /// The items of the frame that wait for the end (`run_items`, the post).
    Items,
    /// `CommandEncoder::finish`: the encoding of the frame's passes.
    Finish,
    /// `Queue::submit`.
    Submit,
    /// The picture files of the frame.
    Pictures,
    /// The extension's `after_frame`: its loads and its facts of the frame.
    AfterFrame,
    /// The present of the window's frame.
    Present,
}

impl Phase {
    const ALL: [Phase; 7] = [
        Phase::Feed,
        Phase::Items,
        Phase::Finish,
        Phase::Submit,
        Phase::Pictures,
        Phase::AfterFrame,
        Phase::Present,
    ];

    fn name(self) -> &'static str {
        match self {
            Phase::Feed => "feed of the record",
            Phase::Items => "items of the frame",
            Phase::Finish => "finish of the command encoder",
            Phase::Submit => "queue submit",
            Phase::Pictures => "picture files",
            Phase::AfterFrame => "after_frame of the extension",
            Phase::Present => "present",
        }
    }

    fn of(number: u8) -> Option<Phase> {
        Phase::ALL.get(number as usize).copied()
    }
}

/// The frame thread is in `phase` from now: one relaxed store.
pub(crate) fn phase(phase: Phase) {
    PHASE.store(phase as u8, Ordering::Relaxed);
}

/// The last instruction pointers of the frame thread, written by the sampler thread alone and
/// read by the watchdog thread when the step stalls. Nothing here takes a lock.
struct SampleRing {
    slots: [AtomicUsize; SAMPLES_KEPT],
    /// The samples taken in this step; slot `n % SAMPLES_KEPT` holds the sample number `n`.
    count: AtomicU64,
    /// The longest time that one look took, from the suspend to the end of `GetThreadContext`.
    costliest_us: AtomicU64,
    /// The time ([`now_ms`]) at which the look in progress began, or [`NO_LOOK`].
    look_began_ms: AtomicU64,
}

/// `look_began_ms` while no look is in progress.
const NO_LOOK: u64 = u64::MAX;

impl SampleRing {
    const fn new() -> SampleRing {
        SampleRing {
            slots: [const { AtomicUsize::new(0) }; SAMPLES_KEPT],
            count: AtomicU64::new(0),
            costliest_us: AtomicU64::new(0),
            look_began_ms: AtomicU64::new(NO_LOOK),
        }
    }

    /// From the sampler thread only.
    fn push(&self, rip: usize) {
        let n = self.count.load(Ordering::Relaxed);
        self.slots[(n % SAMPLES_KEPT as u64) as usize].store(rip, Ordering::Relaxed);
        self.count.store(n + 1, Ordering::Release);
    }

    /// From the sampler thread only: a new step starts.
    fn reset(&self) {
        self.count.store(0, Ordering::Release);
        self.costliest_us.store(0, Ordering::Relaxed);
    }

    /// The samples now, the oldest first, with the phase.
    fn evidence(&self, phase: u8) -> StallEvidence {
        let n = self.count.load(Ordering::Acquire);
        let held = n.min(SAMPLES_KEPT as u64);
        let mut evidence = StallEvidence {
            phase,
            ..StallEvidence::default()
        };
        for at in n - held..n {
            evidence.push(self.slots[(at % SAMPLES_KEPT as u64) as usize].load(Ordering::Relaxed));
        }
        evidence
    }
}

/// Milliseconds since the first call.
fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

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
        phase(Phase::Feed);
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
    if long_limit() {
        LOADING_SECONDS as u32
    } else {
        STALL_SECONDS as u32
    }
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
/// limit until the helper's next read (`publish`). Guards can nest. Make it the first statement
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

    /// The step in progress has lasted [`SAMPLE_FROM_MS`]: the sampler takes samples.
    fn sampling(&self) -> bool {
        self.stalled_ms >= SAMPLE_FROM_MS
    }
}

/// Starts the watchdog thread for the frame thread, which is the caller.
pub(crate) fn start() {
    // SAFETY: a plain read of the id of this thread.
    let frame_thread = unsafe { GetCurrentThreadId() };
    start_sampler(frame_thread);
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let mut watch = Watch::default();
            while !DISARMED.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(LOOK_MS));
                let look = Look {
                    steps: STEPS.load(Ordering::Relaxed),
                    in_step: IN_STEP.load(Ordering::Acquire),
                    loading: long_limit(),
                };
                // A refusal that came during the look ends the program itself.
                let stall = watch.look(look);
                SAMPLING.store(watch.sampling(), Ordering::Release);
                match stall {
                    Some(seconds) if !DISARMED.load(Ordering::Acquire) => stalled(frame_thread, seconds),
                    _ => {}
                }
            }
        })
        .expect("the watchdog thread");
}

/// Starts the sampler thread: at each look of a slow step ([`SAMPLING`]) it reads the instruction
/// pointer of the frame thread. The thread is its own so that a suspend that waits (the frame
/// thread inside a kernel wait that holds off the suspend) holds up the sampler only: the
/// watchdog thread goes on and ends the program at its limit, and the line of the stall says
/// how long the look had waited.
fn start_sampler(frame_thread: u32) {
    let handle = crate::crash::open_for_samples(frame_thread);
    if handle == 0 {
        return;
    }
    let _ = std::thread::Builder::new().name("watchdog-sampler".into()).spawn(move || {
        while !DISARMED.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(LOOK_MS));
            if !SAMPLING.load(Ordering::Acquire) {
                if RING.count.load(Ordering::Relaxed) != 0 {
                    RING.reset();
                }
                continue;
            }
            let began = Instant::now();
            RING.look_began_ms.store(now_ms(), Ordering::Release);
            let rip = crate::crash::sample_rip(handle);
            RING.look_began_ms.store(NO_LOOK, Ordering::Release);
            RING.costliest_us
                .fetch_max(began.elapsed().as_micros() as u64, Ordering::Relaxed);
            if let Some(rip) = rip {
                RING.push(rip);
            }
        }
    });
}

/// The line that names what the stall left: the phase, the samples (the oldest first, a run of
/// equal samples as one with its count), the longest look, and a look that is still waiting.
/// `place` names an address (`module+offset`).
fn evidence_line(
    evidence: &StallEvidence,
    costliest_us: u64,
    waiting_ms: Option<u64>,
    place: impl Fn(usize) -> String,
) -> String {
    let phase = Phase::of(evidence.phase).map_or("unknown", Phase::name);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &rip in evidence.held() {
        match runs.last_mut() {
            Some((last, times)) if *last == rip => *times += 1,
            _ => runs.push((rip, 1)),
        }
    }
    let places: Vec<String> = runs
        .iter()
        .map(|&(rip, times)| {
            if times == 1 {
                place(rip)
            } else {
                format!("{} x{times}", place(rip))
            }
        })
        .collect();
    let mut line = format!("stall evidence: phase {phase}; {} samples of the frame thread, the oldest first: [{}]; {} different; the longest look took {costliest_us} us", evidence.count, places.join(", "), distinct(evidence.held()));
    if let Some(waiting) = waiting_ms {
        line.push_str(&format!("; a look has waited {waiting} ms for the frame thread to suspend"));
    }
    line
}

/// How many different values `samples` holds.
fn distinct(samples: &[usize]) -> usize {
    let mut seen: Vec<usize> = samples.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

/// The frame thread stalled: the exit announced to the plugin, the log lines, then the dump
/// and the end of the program.
/// The lines are written before the frame thread is suspended: a thread suspended inside an
/// allocation holds its lock. They are written without the lock of stdout
/// ([`crate::log_pipe::write_direct`]): a frame thread held inside a print holds that lock,
/// and a print of the watchdog would wait for it, with no dump and no end.
fn stalled(frame_thread: u32, seconds: u64) -> ! {
    // First the exit for the plugin: a dump that hangs is then ended with the code 30, not 31.
    crate::live::announce_exit(EXIT_STALLED);
    let evidence = RING.evidence(PHASE.load(Ordering::Relaxed));
    write_stall_lines(seconds, &evidence);
    crate::crash::stall_dump_and_end(frame_thread, EXIT_STALLED, evidence)
}

/// The two lines of a stall: the stall, then the evidence of the samples.
fn write_stall_lines(seconds: u64, evidence: &StallEvidence) {
    let stall = format!("stalled for {seconds} s: the frame thread made no progress in a step; a minidump, then the program ends with the exit code {EXIT_STALLED}\n");
    crate::log_pipe::write_direct(stall.as_bytes());
    let began = RING.look_began_ms.load(Ordering::Acquire);
    let waiting = (began != NO_LOOK).then(|| now_ms().saturating_sub(began));
    let line = format!(
        "{}\n",
        evidence_line(
            evidence,
            RING.costliest_us.load(Ordering::Relaxed),
            waiting,
            crate::crash::place
        )
    );
    crate::log_pipe::write_direct(line.as_bytes());
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

    /// The lines of a stall are written while another thread holds the lock of stdout: a frame
    /// thread held inside a print holds it. The test runs alone in a child process without the
    /// harness's capture of the output, which a print of a test uses in place of stdout.
    #[test]
    fn the_stall_lines_do_not_wait_for_the_lock_of_stdout() {
        crate::crash::tests::in_child_of(
            module_path!(),
            "the_stall_lines_do_not_wait_for_the_lock_of_stdout_in_this_process",
        );
    }

    #[test]
    #[ignore = "holds the lock of stdout: run alone by the_stall_lines_do_not_wait_for_the_lock_of_stdout"]
    fn the_stall_lines_do_not_wait_for_the_lock_of_stdout_in_this_process() {
        let held = std::io::stdout().lock();
        let (done, waiter) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            write_stall_lines(5, &StallEvidence::default());
            let _ = done.send(());
        });
        let returned = waiter.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(held);
        assert!(returned, "the lines of a stall waited for the lock of stdout");
    }

    #[test]
    fn a_step_that_stays_for_the_limit_is_a_stall() {
        let mut watch = Watch::default();
        let stuck = Look {
            steps: 7,
            in_step: true,
            loading: false,
        };
        // The first look sees the step; the next 19 looks are under 5 s; the 21st reaches it.
        assert_eq!(looks(&mut watch, 20, stuck), None);
        assert_eq!(watch.look(stuck), Some(5));
    }

    #[test]
    fn steps_that_move_or_end_are_no_stall() {
        let mut watch = Watch::default();
        for steps in 0..100 {
            assert_eq!(
                watch.look(Look {
                    steps,
                    in_step: true,
                    loading: false
                }),
                None
            );
        }
        // Between the steps nothing is in progress: a long wait for the game is no stall.
        assert_eq!(
            looks(
                &mut watch,
                100,
                Look {
                    steps: 100,
                    in_step: false,
                    loading: false
                }
            ),
            None
        );
    }

    #[test]
    fn a_load_in_progress_gives_the_step_30_s() {
        let mut watch = Watch::default();
        let loading = Look {
            steps: 1,
            in_step: true,
            loading: true,
        };
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
        // A step starts in the phase of the feed, whatever phase the step before ended in.
        phase(Phase::Present);
        drop(Step::begin());
        assert_eq!(PHASE.load(Ordering::Relaxed), Phase::Feed as u8);
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

        // The time of a long part does not count after it, with the real count and rule: 25
        // looks (over 6 s) inside a guard, then a look after its drop. The short limit counts
        // from the drop: a real hang after the guard reaches the 5 s at the 21st look. The same
        // for the end of a load.
        let look = || Look {
            steps: STEPS.load(Ordering::Relaxed),
            in_step: true,
            loading: long_limit(),
        };
        let mut watch = Watch::default();
        let guard = LongStep::begin();
        assert_eq!(looks_of(&mut watch, 25, &look), None);
        drop(guard);
        assert_eq!(looks_of(&mut watch, 20, &look), None);
        assert_eq!(watch.look(look()), Some(5));
        let mut watch = Watch::default();
        loading(true);
        assert_eq!(looks_of(&mut watch, 25, &look), None);
        loading(false);
        assert_eq!(looks_of(&mut watch, 20, &look), None);
        assert_eq!(watch.look(look()), Some(5));
    }

    fn looks_of(watch: &mut Watch, count: usize, look: &dyn Fn() -> Look) -> Option<u64> {
        (0..count).map(|_| watch.look(look())).last().flatten()
    }

    #[test]
    fn a_step_is_sampled_from_one_second_and_never_before() {
        let mut watch = Watch::default();
        let slow = Look {
            steps: 7,
            in_step: true,
            loading: false,
        };
        // The first look sees the step (0 ms); each look after it adds 250 ms.
        assert_eq!(watch.look(slow), None);
        for _ in 0..3 {
            assert_eq!(watch.look(slow), None);
            assert!(!watch.sampling(), "under 1 s: {} ms", watch.stalled_ms);
        }
        assert_eq!(watch.look(slow), None);
        assert!(watch.sampling(), "1 s: {} ms", watch.stalled_ms);
        // A step that ends or moves stops the sampling at once, so that a quick step after a slow
        // one is never sampled.
        assert_eq!(
            watch.look(Look {
                steps: 8,
                in_step: true,
                loading: false
            }),
            None
        );
        assert!(!watch.sampling());
        for _ in 0..3 {
            watch.look(Look {
                steps: 8,
                in_step: true,
                loading: false,
            });
        }
        assert!(!watch.sampling());
        watch.look(Look {
            steps: 8,
            in_step: false,
            loading: false,
        });
        assert!(!watch.sampling());
    }

    #[test]
    fn the_ring_keeps_the_last_samples_oldest_first() {
        let ring = SampleRing::new();
        assert_eq!(ring.evidence(2).held(), &[] as &[usize]);
        for rip in 1..=5 {
            ring.push(rip);
        }
        assert_eq!(ring.evidence(2).held(), &[1, 2, 3, 4, 5]);
        for rip in 6..=20 {
            ring.push(rip);
        }
        let evidence = ring.evidence(3);
        assert_eq!(evidence.held(), &(5..=20).collect::<Vec<usize>>()[..]);
        assert_eq!((evidence.phase, evidence.count), (3, SAMPLES_KEPT));
        // The next step starts again from nothing, and the costliest look is forgotten.
        ring.costliest_us.store(77, Ordering::Relaxed);
        ring.reset();
        assert_eq!(ring.evidence(0).held(), &[] as &[usize]);
        assert_eq!(ring.costliest_us.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_phase_numbers_name_every_phase_once() {
        for (at, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(*phase as usize, at);
            assert_eq!(Phase::of(at as u8), Some(*phase));
        }
        assert_eq!(Phase::of(Phase::ALL.len() as u8), None);
        let mut names: Vec<&str> = Phase::ALL.iter().map(|phase| phase.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Phase::ALL.len());
    }

    #[test]
    fn the_stall_line_names_the_phase_and_folds_a_run_of_the_same_place() {
        let place = |rip: usize| format!("numidium+{rip:#x}");
        let ring = SampleRing::new();
        for rip in [0x10, 0x10, 0x10, 0x20, 0x10] {
            ring.push(rip);
        }
        let line = evidence_line(&ring.evidence(Phase::Finish as u8), 42, None, place);
        assert_eq!(
            line,
            "stall evidence: phase finish of the command encoder; 5 samples of the frame thread, the oldest first: [numidium+0x10 x3, numidium+0x20, numidium+0x10]; 2 different; the longest look took 42 us"
        );
        // A look that is still waiting, and a phase number that no phase has.
        let line = evidence_line(
            &StallEvidence {
                phase: 200,
                ..StallEvidence::default()
            },
            0,
            Some(3100),
            place,
        );
        assert_eq!(line, "stall evidence: phase unknown; 0 samples of the frame thread, the oldest first: []; 0 different; the longest look took 0 us; a look has waited 3100 ms for the frame thread to suspend");
    }
}
