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
        if crate::mark_census::on() && crate::shadow::on_main_thread() {
            crate::mark_census::note_update_root(node);
        }
    }
}

#[inline]
pub fn leave() {
    DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
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
    format!("update root: {} root calls of the downward pass", ROOTS.load(Ordering::Relaxed))
}
