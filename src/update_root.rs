//! The root of each subtree update, from a light detour on `NiNode::UpdateDownwardPass`
//! (D7 step 3, 2026-10-19). MWSE replaces `NiAVObject::Update` whole, so that function
//! cannot be hooked; but the engine's function and MWSE's replacement both start the root's
//! downward pass through the virtual table, and the pass recurses through this function for
//! every node below: the outermost call on a thread is the update's root (engine facts R5e).
//!
//! A port on the function cost 0.24 ms a frame at the hold (entry 36): the port's dispatch,
//! paid for every node of every updated subtree. This detour keeps a depth per thread and
//! calls the engine's code through its trampoline; at depth 0 it counts the root and, with
//! the census on and on the main thread, notes it (`mark_census::note_update_root`). The
//! animation node's own pass (`ports/animation.rs`) enters the same depth, since an update
//! can start at one. `NIRENDER_ROOT_HOOK=0` leaves the detour out (the A/B of its cost).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const UPDATE_DOWNWARD_PASS: usize = 0x6C8B40;
/// The function's first two instructions, `mov al, [esp+8]` and `sub esp, 8`: 7 bytes with
/// no relative target (the plugin's code table, offset 0x162CC).
const HEAD: [u8; 7] = [0x8A, 0x44, 0x24, 0x08, 0x83, 0xEC, 0x08];

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The root calls (depth 0) on any thread.
static ROOTS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static DEPTH: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// D7 step 3 (2026-10-19): the root mark. On the main thread, with the mark on, each root
/// of a subtree update goes into this list, which the table walk drains once a frame: the
/// walk descends from a root in the view and defers the shapes of one out of the view, as
/// it does for the actors' roots of D6. A per-shape move mark made inside a pass is then
/// not needed (`in_pass`); one made at depth 0, an update that starts at a shape, stays.
/// The list is a fixed array (the detour never allocates); a full array sets `lost`, and the
/// next walk is a full pass. `NIRENDER_ROOT_MARK=1` turns the mark on; off by default since
/// 2026-10-19 (the owner: the default walk is the engine's, decisions.md; the mark stays a
/// switch for the comparison). Off, the detour only counts.
const MARK_CAPACITY: usize = 16384;
static mut MARKS: [usize; MARK_CAPACITY] = [0; MARK_CAPACITY];
static mut MARK_LEN: usize = 0;
static MARK_LOST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static MARKS_PUSHED: AtomicU64 = AtomicU64::new(0);
static MARKS_LOST_RUNS: AtomicU64 = AtomicU64::new(0);
/// The per-shape move marks not made because the shape's update ran inside a pass.
static SHAPE_MARKS_SKIPPED: AtomicU64 = AtomicU64::new(0);

pub fn mark_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| on() && std::env::var("NIRENDER_ROOT_MARK").is_ok_and(|v| v == "1"))
}

/// True inside a downward pass on this thread: a per-shape update seen now is below a root
/// that is marked.
#[inline]
pub fn in_pass() -> bool {
    DEPTH.with(|d| d.get() > 0)
}

pub fn note_shape_mark_skipped() {
    SHAPE_MARKS_SKIPPED.fetch_add(1, Ordering::Relaxed);
}

/// Enters a downward pass on this thread; at depth 0 the node is a subtree update's root.
#[inline]
pub unsafe fn enter(node: usize) {
    let depth = DEPTH.with(|d| {
        let v = d.get();
        d.set(v + 1);
        v
    });
    if depth == 0 {
        ROOTS.fetch_add(1, Ordering::Relaxed);
        if crate::shadow::on_main_thread() {
            if mark_on() {
                MARKS_PUSHED.fetch_add(1, Ordering::Relaxed);
                let len = *core::ptr::addr_of!(MARK_LEN);
                if len < MARK_CAPACITY {
                    (*core::ptr::addr_of_mut!(MARKS))[len] = node;
                    *core::ptr::addr_of_mut!(MARK_LEN) = len + 1;
                } else {
                    MARK_LOST.store(true, Ordering::Relaxed);
                }
            }
            if crate::mark_census::on() {
                crate::mark_census::note_update_root(node);
            }
        }
    }
}

#[inline]
pub fn leave() {
    DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
}

/// Moves the roots marked since the last drain into `out` (with repeats: a root updated
/// several times a frame appears as often); true when some were lost. Main thread only.
pub unsafe fn drain(out: &mut Vec<usize>) -> bool {
    out.clear();
    let len = *core::ptr::addr_of!(MARK_LEN);
    out.extend_from_slice(core::slice::from_raw_parts(core::ptr::addr_of!(MARKS) as *const usize, len));
    *core::ptr::addr_of_mut!(MARK_LEN) = 0;
    let lost = MARK_LOST.swap(false, Ordering::Relaxed);
    if lost {
        MARKS_LOST_RUNS.fetch_add(1, Ordering::Relaxed);
    }
    lost
}

unsafe extern "thiscall" fn detour(this: usize, time: f32, update_controllers: u8, update_children: u8) {
    enter(this);
    let original: unsafe extern "thiscall" fn(usize, f32, u8, u8) = core::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
    original(this, time, update_controllers, update_children);
    leave();
}

pub fn on() -> bool {
    ORIGINAL.load(Ordering::Relaxed) != 0
}

pub unsafe fn install() {
    if std::env::var("NIRENDER_ROOT_HOOK").is_ok_and(|v| v == "0") {
        crate::log!("update root: off (NIRENDER_ROOT_HOOK=0); no detour on NiNode::UpdateDownwardPass");
        return;
    }
    match crate::hook::install_raw(UPDATE_DOWNWARD_PASS, &HEAD, &[], detour as *const () as usize, 3, crate::hook::Cleanup::Callee) {
        Ok(trampoline) => {
            ORIGINAL.store(trampoline, Ordering::Relaxed);
            crate::log!("update root: a light detour on NiNode::UpdateDownwardPass ({UPDATE_DOWNWARD_PASS:#x}) counts the root of each subtree update");
        }
        Err(error) => crate::log!("update root: no detour on NiNode::UpdateDownwardPass: {error}"),
    }
}

pub fn report() -> String {
    if !on() {
        return String::from("update root: off");
    }
    format!(
        "update root: {} root calls of the downward pass; the root mark {}: roots marked {}, walks that lost marks {}, per-shape marks not made inside a pass {}",
        ROOTS.load(Ordering::Relaxed),
        if mark_on() { "on" } else { "off" },
        MARKS_PUSHED.load(Ordering::Relaxed),
        MARKS_LOST_RUNS.load(Ordering::Relaxed),
        SHAPE_MARKS_SKIPPED.load(Ordering::Relaxed)
    )
}
