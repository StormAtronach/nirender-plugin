//! The end of the flush of the engine's main sorted queue, from MWSE's event, not from the
//! draws. The helper draws the table's sorted blends with the engine's sorted blends; it
//! learns the place of the engine's flush here.
//!
//! `NiCamera::Click` draws the queue of its accumulator at its end (0x6CC895, the call of
//! `FinishAccumulating`), unless its argument keeps the queue open for a later Click of the
//! same camera: the world Click keeps it open when the water is shown or the shadow manager is
//! on, and the water's Click draws it (engine-rules R19, R22 of the skill wgpu-renderer-dev).
//! MWSE's event `MWSE_NATIVE_EVENT_ACCUMULATOR_FINISHED` comes after each such draw. For the
//! accumulator of the current Click (`ports::base::is_main_accumulator`), the frame event
//! `EVENT_QUEUE_FLUSHED` goes into the stream.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::main_thread::MainThread;

static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The draws of an accumulator's queue that MWSE reported, and the events of the main queue.
static FINISHED: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU64 = AtomicU64::new(0);

/// True when the plugin listens to MWSE's event: the stream then carries the event, and its
/// header says so (`capture::HEADER_QUEUE_FLUSH`).
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// Listens to MWSE's event. A refusal (an MWSE without the event) leaves the stream without
/// the event: the helper then places the flush as an older capture has it.
pub unsafe fn install() {
    if installed() {
        return;
    }
    if let Err(error) = crate::mwse_native::subscribe(crate::mwse_native::NATIVE_EVENT_ACCUMULATOR_FINISHED, accumulator_finished) {
        crate::log!("queue flush: {error}: the helper places the flush as before");
        return;
    }
    INSTALLED.store(true, Ordering::Relaxed);
    crate::log!("queue flush: MWSE reports the end of each draw of an accumulator's queue");
}

/// The report line.
pub fn report() -> String {
    format!("queue flush: {} draws of a queue, {} events of the main queue", FINISHED.load(Ordering::Relaxed), EVENTS.load(Ordering::Relaxed))
}

unsafe extern "C" fn accumulator_finished(_user: *mut core::ffi::c_void, packet: *const core::ffi::c_void) {
    let packet = &*(packet as *const crate::mwse_native::AccumulatorFinishedV1);
    FINISHED.fetch_add(1, Ordering::Relaxed);
    let Some(t) = MainThread::here() else { return };
    if !crate::ports::base::is_main_accumulator(&t, packet.accumulator as usize) {
        return;
    }
    EVENTS.fetch_add(1, Ordering::Relaxed);
    let _lock = crate::sync::device_lock();
    crate::capture::frame_event(crate::link::EVENT_QUEUE_FLUSHED, 0, 0);
}
