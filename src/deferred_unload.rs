//! The unload of a cell deferred to the main thread (plan C27, option L1, 2026-10-18).
//!
//! The engine's cell worker frees scene objects in one place: `RecordsHandler::unloadCell`
//! (0x4BAB00), called when it evicts a cell outside the 5x5 background area from the cell
//! buffer (0x48FA00, and `registerLoadedExteriorCell` 0x489EC0). Every other destruction of
//! a scene object happens on the main thread (engine facts LD4). The worker runs beside the
//! main thread's frames during the preload near a cell border, so those evictions freed
//! objects while the main thread read them: the crashes of the manual review of 2026-10-18.
//!
//! Here a hook on `unloadCell` queues the cell when the call comes from another thread and
//! returns without unloading. The main thread runs the queued unloads at each Present, one
//! cell a frame, so the cost of the destructors stays small in a frame (each is timed; see
//! `report`). Then every destruction of a scene object is on the main thread, the plugin's
//! ports see it, and nothing frees an object while a frame reads it.
//!
//! A cell the engine loaded again before its deferred unload (the player turned back) stays:
//! the unload is skipped when the cell is in the exterior cell buffer again, or when its
//! statics node is gone already. The engine shifted the buffer when it evicted the cell, so
//! the cell's objects only live longer; nothing else of the engine's state differs [I].
//!
//! `NIRENDER_DEFER_UNLOAD=0` turns the hook off (the A/B).

use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::mem::rd;

/// `RecordsHandler::unloadCell(this, cell)`, `__thiscall`, one stack argument.
const UNLOAD_CELL: usize = 0x4BAB00;
/// `dataHandler` (0x7C67E0), its `exteriorCellBuffer` (+0xB4, `Cell**`) and the buffer's
/// size (`global_exteriorCellBufferSize` 0x7C9B48).
const DATA_HANDLER: usize = 0x7C67E0;
const EXTERIOR_CELL_BUFFER: usize = 0xB4;
const EXTERIOR_CELL_BUFFER_SIZE: usize = 0x7C9B48;
/// `Cell::staticObjectsRoot` (+0x2C): 0 after an unload.
const CELL_STATICS_ROOT: usize = 0x2C;

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The queued unloads: the records handler and the cell of each call.
static QUEUE: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
static QUEUED: AtomicU64 = AtomicU64::new(0);
static UNLOADED: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static TICKS: AtomicI64 = AtomicI64::new(0);
static LONGEST: AtomicI64 = AtomicI64::new(0);

unsafe extern "thiscall" fn detour(this: usize, cell: usize) -> u32 {
    if crate::shadow::on_main_thread() {
        return unload(this, cell);
    }
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push((this, cell));
        QUEUED.fetch_add(1, Ordering::Relaxed);
    }
    1
}

/// The engine's function.
unsafe fn unload(this: usize, cell: usize) -> u32 {
    let original: unsafe extern "thiscall" fn(usize, usize) -> u32 = core::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
    original(this, cell)
}

/// True when the engine loaded the cell again since its eviction: it is in the exterior cell
/// buffer, or it has no statics node (unloaded already).
unsafe fn loaded_again(cell: usize) -> bool {
    if rd::<usize>(cell, CELL_STATICS_ROOT) == 0 {
        return true;
    }
    let data_handler: usize = rd(DATA_HANDLER, 0);
    if data_handler == 0 {
        return false;
    }
    let buffer: usize = rd(data_handler, EXTERIOR_CELL_BUFFER);
    let size: u32 = rd(EXTERIOR_CELL_BUFFER_SIZE, 0);
    buffer != 0 && (0..size as usize).any(|i| rd::<usize>(buffer, i * 4) == cell)
}

pub unsafe fn install() {
    if std::env::var("NIRENDER_DEFER_UNLOAD").is_ok_and(|v| v == "0") {
        crate::log!("deferred unload: off (NIRENDER_DEFER_UNLOAD=0); the cell worker unloads cells on its own thread");
        return;
    }
    // The function is not in the plugin's function table (it is not of the renderer): the
    // hook takes its first two instructions as the IDA database has them, `push ebx; mov
    // ebx, [esp+8]` (5 bytes, no relative target).
    const HEAD: [u8; 5] = [0x53, 0x8B, 0x5C, 0x24, 0x08];
    match crate::hook::install_raw(UNLOAD_CELL, &HEAD, &[], detour as *const () as usize, 1, crate::hook::Cleanup::Callee) {
        Ok(trampoline) => {
            ORIGINAL.store(trampoline, Ordering::Relaxed);
            crate::log!("deferred unload: the cells the cell worker evicts are unloaded on the main thread, one a frame (RecordsHandler::unloadCell {UNLOAD_CELL:#x})");
        }
        Err(error) => crate::log!("deferred unload: no hook on RecordsHandler::unloadCell: {error}"),
    }
}

/// The main thread unloads one queued cell. At each Present.
pub unsafe fn run() {
    if ORIGINAL.load(Ordering::Relaxed) == 0 || !crate::shadow::on_main_thread() {
        return;
    }
    let next = match QUEUE.lock() {
        Ok(mut queue) => {
            if queue.is_empty() {
                return;
            }
            Some(queue.remove(0))
        }
        Err(_) => None,
    };
    let Some((this, cell)) = next else { return };
    if loaded_again(cell) {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let start = crate::capture::ticks();
    unload(this, cell);
    let ticks = crate::capture::ticks() - start;
    TICKS.fetch_add(ticks, Ordering::Relaxed);
    LONGEST.fetch_max(ticks, Ordering::Relaxed);
    if UNLOADED.fetch_add(1, Ordering::Relaxed) == 0 {
        crate::log!("deferred unload: the first cell unloaded on the main thread, {:.3} ms", ms(ticks));
    }
}

fn ms(ticks: i64) -> f64 {
    ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64
}

pub fn report() -> String {
    let unloaded = UNLOADED.load(Ordering::Relaxed);
    let total = TICKS.load(Ordering::Relaxed);
    format!(
        "deferred unload (L1): {} cells queued by the cell worker, {} unloaded on the main thread ({:.3} ms in all, {:.3} ms a cell, the longest {:.3} ms), {} skipped (loaded again), {} waiting",
        QUEUED.load(Ordering::Relaxed),
        unloaded,
        ms(total),
        if unloaded == 0 { 0.0 } else { ms(total) / unloaded as f64 },
        ms(LONGEST.load(Ordering::Relaxed)),
        SKIPPED.load(Ordering::Relaxed),
        QUEUE.lock().map_or(0, |q| q.len())
    )
}
